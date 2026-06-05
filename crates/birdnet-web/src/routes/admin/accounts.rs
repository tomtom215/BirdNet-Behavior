//! Accounts & sessions surface (O-15).
//!
//! Page at `/admin/accounts` with three cards: active sessions, users
//! roster, and a 6-row preview of the audit log. Mutating endpoints
//! cover invite/delete on users, rotate-password on any user, and revoke
//! on sessions; the handlers hit the stores in [`birdnet_db::accounts`].
//!
//! Access is gated centrally, not per handler: the O-14 cookie middleware
//! authenticates every `/admin` request and the O-15 RBAC check restricts
//! writes to the `admin` role (wire flipped in #96, reconciled in #112), so
//! these handlers assume an authenticated admin and carry no auth of their
//! own. The request-time user comes from the [`RequestUser`] extractor; the
//! seed `admin` row is created by the schema migration.
//!
//! Each mutating handler emits an OOB toast (success/warn) using the
//! O-18 helper so the operator sees the result without a full reload.

use std::fmt::Write as _;

use axum::Form;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::Deserialize;

use birdnet_db::accounts::{
    self, AccountsError, AuditEntry, AuditLog, Role, Session, SessionStore, User, UserStore,
};

use super::admin_shell;
use crate::auth_middleware::RequestUser;
use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

const ACCOUNTS_TEMPLATE: &str = include_str!("../../../templates/admin_accounts.html");
const AUDIT_PREVIEW_LIMIT: usize = 6;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/accounts", get(accounts_page))
        .route("/admin/accounts/users", post(create_user))
        .route(
            "/admin/accounts/users/{id}",
            delete(remove_user).post(set_password),
        )
        .route(
            "/admin/accounts/sessions/{id}",
            delete(revoke_session_handler),
        )
        .route(
            "/admin/accounts/sessions/revoke-others",
            post(revoke_others_handler),
        )
        .route("/admin/audit", get(audit_full_page))
}

// ───────────────────────────────────────────────────────────────────────────
// GET /admin/accounts
// ───────────────────────────────────────────────────────────────────────────

async fn accounts_page(State(state): State<AppState>, request_user: RequestUser) -> Html<String> {
    let current_session_id = request_user.session_id.clone();
    let body = state.with_db(|conn| -> Result<String, AccountsError> {
        let current_user = conn.find_user(request_user.user.id)?;
        let users = conn.list_users()?;
        let sessions = conn.list_sessions(current_user.id)?;
        let audit = conn.recent(AUDIT_PREVIEW_LIMIT)?;
        Ok(render_body(
            &current_user,
            &users,
            &sessions,
            &audit,
            &current_session_id,
        ))
    });
    let body = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "accounts page render failed");
            render_error("Accounts data could not be loaded.")
        }
    };
    Html(admin_shell("Accounts", "accounts", &body))
}

fn render_body(
    current_user: &User,
    users: &[User],
    sessions: &[Session],
    audit: &[AuditEntry],
    current_session_id: &str,
) -> String {
    let session_rows = render_session_rows(sessions, current_session_id);
    let user_rows = render_user_rows(users);
    let audit_rows = render_audit_rows(audit, users);
    let display_name = current_user
        .label
        .clone()
        .unwrap_or_else(|| current_user.username.clone());

    ACCOUNTS_TEMPLATE
        .replace("{{session_rows}}", &session_rows)
        .replace("{{user_rows}}", &user_rows)
        .replace("{{audit_rows}}", &audit_rows)
        .replace("{{current_user}}", &escape_html(&display_name))
}

fn render_session_rows(sessions: &[Session], current_session_id: &str) -> String {
    if sessions.is_empty() {
        return String::from(
            r#"<li class="account-sessions__empty"><span class="bnb-meta">No active sessions yet. Sign in once to seed this list.</span></li>"#,
        );
    }
    let mut out = String::new();
    for s in sessions {
        let id_tail = s.id.chars().rev().take(4).collect::<String>();
        let id_tail: String = id_tail.chars().rev().collect();
        let agent = s.user_agent.as_deref().unwrap_or("Unknown device");
        let is_current = s.id == current_session_id;
        let current_marker = if is_current { "true" } else { "false" };
        let current_pill = if is_current {
            r#"<span class="session-pill current" title="The browser you're using right now">This device</span>"#
        } else {
            ""
        };
        let _ = write!(
            out,
            r##"<li data-current="{current_marker}">
  <span class="session-mark" aria-hidden="true"></span>
  <div>
    <div class="session-label">{label}{current_pill}</div>
    <div class="session-meta">last seen {last}</div>
  </div>
  <span class="session-id">#{id_tail}</span>
  <button class="bnb-btn ghost"
          data-confirm-action="hx-delete"
          data-confirm-url="/admin/accounts/sessions/{id}"
          data-confirm-title="Sign out this device"
          data-confirm-body="That session ends immediately. Re-signing in is required from that browser."
          data-confirm-confirm-label="Sign out"
          data-confirm-style="warn"
          hx-delete="/admin/accounts/sessions/{id}"
          hx-target="#session-list"
          hx-swap="innerHTML">Sign out →</button>
</li>"##,
            label = escape_html(agent),
            last = escape_html(&s.last_seen),
            id = escape_html(&s.id),
        );
    }
    out
}

/// Inline "reset password" control for a user row.
///
/// Posts a new password to the live [`set_password`] handler
/// (`POST /admin/accounts/users/{id}`), replacing the dead button that used to
/// post to a non-existent `…/password-reset-stub` route. `hx-swap="none"`
/// because the handler answers with an out-of-band toast, not a row fragment;
/// the 10-char minimum mirrors the server-side check so the browser blocks the
/// obvious case before the round-trip.
fn password_reset_form(id: i64) -> String {
    format!(
        r#"<form class="user-reset acct-reset" hx-post="/admin/accounts/users/{id}" hx-swap="none" autocomplete="off">
  <input type="password" name="password" minlength="10" required placeholder="New password (min 10)" autocomplete="new-password">
  <button type="submit" class="bnb-btn ghost">Reset password</button>
</form>"#
    )
}

fn render_user_rows(users: &[User]) -> String {
    if users.is_empty() {
        return String::from(
            r#"<li class="account-users__empty"><span class="bnb-meta">No users — the admin row is created by the schema migration.</span></li>"#,
        );
    }
    let mut out = String::new();
    for u in users {
        let pill_class = match u.role {
            Role::Admin => "user-pill admin",
            Role::Viewer => "user-pill viewer",
        };
        let pill_label = match u.role {
            Role::Admin => "ADMIN",
            Role::Viewer => "VIEWER",
        };
        let display = u.label.clone().unwrap_or_else(|| u.username.clone());
        let id = u.id;
        let reset = password_reset_form(id);
        let actions = if u.username == "admin" {
            // Seed admin can rotate its password but not be removed/disabled.
            reset
        } else {
            format!(
                r##"<div class="acct-actions">
  {reset}
  <button class="bnb-btn ghost"
          data-confirm-action="hx-delete"
          data-confirm-url="/admin/accounts/users/{id}"
          data-confirm-title="Remove user"
          data-confirm-body="That account is removed and every session for it ends immediately."
          data-confirm-confirm-label="Remove"
          data-confirm-style="danger"
          hx-delete="/admin/accounts/users/{id}"
          hx-target="#user-list"
          hx-swap="innerHTML">Remove</button>
</div>"##,
            )
        };
        let _ = write!(
            out,
            r#"<li>
  <span class="{pill_class}">{pill_label}</span>
  <div>
    <div class="user-name">{name}</div>
    <div class="user-sub mono">{username} · joined {created}</div>
  </div>
  {actions}
</li>"#,
            name = escape_html(&display),
            username = escape_html(&u.username),
            created = escape_html(&u.created_at),
        );
    }
    out
}

fn render_audit_rows(audit: &[AuditEntry], users: &[User]) -> String {
    if audit.is_empty() {
        return String::from(
            r#"<li class="account-audit__empty"><span class="bnb-meta">No admin actions yet. Settings changes and rule edits appear here as they happen.</span></li>"#,
        );
    }
    let mut out = String::new();
    for e in audit {
        let who = e
            .user_id
            .and_then(|id| users.iter().find(|u| u.id == id))
            .map_or("system", |u| u.username.as_str());
        let target = e
            .target
            .as_ref()
            .map(|t| format!(r#"<span class="target">{}</span>"#, escape_html(t)))
            .unwrap_or_default();
        let _ = write!(
            out,
            r#"<li>
  <span class="audit-who">{who}</span>
  <span class="audit-action">{action}{target}</span>
  <span class="audit-when">{when}</span>
</li>"#,
            who = escape_html(who),
            action = escape_html(&e.action),
            target = target,
            when = escape_html(&e.at),
        );
    }
    out
}

fn render_error(message: &str) -> String {
    format!(
        r#"<div data-screen-label="Accounts">
  <header class="page-head">
    <div>
      <div class="bnb-eyebrow">Admin · access</div>
      <h1 class="display acct-h1">Accounts &amp; sessions</h1>
    </div>
  </header>
  <section class="bnb-card pad acct-error">
    <strong>Error.</strong> {msg}
  </section>
</div>"#,
        msg = escape_html(message)
    )
}

// ───────────────────────────────────────────────────────────────────────────
// POST /admin/accounts/users — invite a viewer
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateUserForm {
    username: String,
    password: String,
    #[serde(default)]
    label: Option<String>,
}

async fn create_user(State(state): State<AppState>, Form(form): Form<CreateUserForm>) -> Response {
    if form.password.len() < 10 {
        return toast::oob_only(Toast::error("Password must be at least 10 characters."))
            .into_response();
    }

    let pwd_argon2 = match accounts::hash_password(&form.password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "hash_password failed in create_user");
            return toast::oob_only(Toast::error(
                "Could not hash the password. See server logs.",
            ))
            .into_response();
        }
    };

    let result = state.with_db(|conn| {
        conn.create_user(
            form.username.trim(),
            &pwd_argon2,
            Role::Viewer,
            form.label.as_deref(),
        )
    });

    match result {
        Ok(_) => {
            // Re-render the full user list so the new row appears.
            let users = state.with_db(UserStore::list_users).unwrap_or_default();
            let body = Html(render_user_rows(&users));
            toast::with(
                body,
                Toast::success(format!(
                    "Invited {}. They can sign in now with that password.",
                    form.username
                )),
            )
            .into_response()
        }
        Err(AccountsError::Conflict(_)) => toast::oob_only(Toast::error(format!(
            "Username \"{}\" is already taken.",
            form.username
        )))
        .into_response(),
        Err(AccountsError::Invalid(msg)) => toast::oob_only(Toast::error(msg)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "create_user failed");
            toast::oob_only(Toast::error("Could not create the user. See server logs."))
                .into_response()
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// DELETE /admin/accounts/users/{id}
// ───────────────────────────────────────────────────────────────────────────

async fn remove_user(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let result = state.with_db(|conn| conn.delete_user(id));
    match result {
        Ok(()) => {
            let users = state.with_db(UserStore::list_users).unwrap_or_default();
            let body = Html(render_user_rows(&users));
            toast::with(body, Toast::success("User removed.")).into_response()
        }
        Err(AccountsError::Invalid(msg)) => toast::oob_only(Toast::warn(msg)).into_response(),
        Err(AccountsError::NotFound(_)) => {
            toast::oob_only(Toast::warn("User no longer exists.")).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "remove_user failed");
            toast::oob_only(Toast::error("Could not remove the user.")).into_response()
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// POST /admin/accounts/users/{id}  (rotate password)
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PasswordForm {
    password: String,
}

async fn set_password(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<PasswordForm>,
) -> Response {
    if form.password.len() < 10 {
        return toast::oob_only(Toast::error("Password must be at least 10 characters."))
            .into_response();
    }
    let pwd_argon2 = match accounts::hash_password(&form.password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "hash_password failed in set_password");
            return toast::oob_only(Toast::error(
                "Could not hash the password. See server logs.",
            ))
            .into_response();
        }
    };
    let result = state.with_db(|conn| conn.set_password(id, &pwd_argon2));
    match result {
        Ok(()) => toast::oob_only(Toast::success("Password rotated.")).into_response(),
        Err(AccountsError::NotFound(_)) => {
            toast::oob_only(Toast::warn("User no longer exists.")).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "set_password failed");
            toast::oob_only(Toast::error("Could not rotate the password.")).into_response()
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// DELETE /admin/accounts/sessions/{id}
// ───────────────────────────────────────────────────────────────────────────

async fn revoke_session_handler(
    State(state): State<AppState>,
    request_user: RequestUser,
    Path(id): Path<String>,
) -> Response {
    let result = state.with_db(|conn| conn.revoke_session(&id));
    if let Err(e) = result {
        tracing::error!(error = %e, "revoke_session failed");
        return toast::oob_only(Toast::error("Could not revoke that session.")).into_response();
    }
    let current_session_id = request_user.session_id.clone();
    let body = state
        .with_db(|conn| -> Result<String, AccountsError> {
            let sessions = conn.list_sessions(request_user.user.id)?;
            Ok(render_session_rows(&sessions, &current_session_id))
        })
        .unwrap_or_else(|_| "<li class=\"account-sessions__empty\">—</li>".to_string());
    toast::with(Html(body), Toast::success("Session signed out.")).into_response()
}

// ───────────────────────────────────────────────────────────────────────────
// POST /admin/accounts/sessions/revoke-others
// ───────────────────────────────────────────────────────────────────────────

async fn revoke_others_handler(
    State(state): State<AppState>,
    request_user: RequestUser,
) -> Response {
    let current_session_id = request_user.session_id.clone();
    let user_id = request_user.user.id;
    let result = state.with_db(|conn| -> Result<usize, AccountsError> {
        conn.revoke_others(user_id, &current_session_id)
    });
    let n = match result {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "revoke_others failed");
            return toast::oob_only(Toast::error("Could not sign out the other devices."))
                .into_response();
        }
    };
    let body = state
        .with_db(|conn| -> Result<String, AccountsError> {
            let sessions = conn.list_sessions(user_id)?;
            Ok(render_session_rows(&sessions, &current_session_id))
        })
        .unwrap_or_else(|_| "<li class=\"account-sessions__empty\">—</li>".to_string());
    let label = if n == 1 {
        "Signed out 1 other device.".to_string()
    } else {
        format!("Signed out {n} other devices.")
    };
    toast::with(Html(body), Toast::success(label)).into_response()
}

// ───────────────────────────────────────────────────────────────────────────
// GET /admin/audit — full log (linked from the accounts page)
// ───────────────────────────────────────────────────────────────────────────

/// Query parameters for the audit-log page. All optional; sensible
/// defaults so a bare `/admin/audit` shows the last 14 days unfiltered.
#[derive(Debug, Deserialize, Default)]
struct AuditParams {
    /// Lower bound, inclusive (`YYYY-MM-DD`).
    #[serde(default)]
    from: Option<String>,
    /// Upper bound, inclusive (`YYYY-MM-DD`).
    #[serde(default)]
    to: Option<String>,
    /// Optional substring filter against the `action` column. The
    /// handler wraps it in `%…%` so the operator can type `rule` and
    /// get every `rule.*` action without writing SQL wildcards.
    #[serde(default)]
    action: Option<String>,
}

/// Default lookback window when `?from` is missing.
const AUDIT_DEFAULT_DAYS: u32 = 14;
/// Hard limit on rows rendered in a single page response.
const AUDIT_PAGE_LIMIT: usize = 500;

async fn audit_full_page(
    State(state): State<AppState>,
    Query(params): Query<AuditParams>,
) -> Html<String> {
    let (from, to) = resolve_audit_range(&params);
    let action_filter = params.action.as_deref().unwrap_or("").trim().to_string();
    let action_like = if action_filter.is_empty() {
        String::new()
    } else {
        format!("%{action_filter}%")
    };

    let body = state.with_db(|conn| -> Result<String, AccountsError> {
        let entries = conn.query(&from, &to, &action_like, AUDIT_PAGE_LIMIT)?;
        let users = conn.list_users()?;
        Ok(render_audit_page(
            &from,
            &to,
            &action_filter,
            &entries,
            &users,
        ))
    });

    let body = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "audit page render failed");
            render_error("Audit log could not be loaded.")
        }
    };
    Html(admin_shell("Audit log", "accounts", &body))
}

/// Resolve the inclusive `(from, to)` date strings for the query. When
/// `params.from` is missing, fall back to "today minus `AUDIT_DEFAULT_DAYS`".
/// `params.to` defaults to today.
fn resolve_audit_range(params: &AuditParams) -> (String, String) {
    let today = today_date_string();
    let to = params
        .to
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&today)
        .to_string();
    let from = params
        .from
        .as_deref()
        .filter(|s| !s.is_empty())
        .map_or_else(
            || date_minus_days(&today, i64::from(AUDIT_DEFAULT_DAYS)),
            ToString::to_string,
        );
    (from, to)
}

/// Current UTC date as `YYYY-MM-DD`.
fn today_date_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let (y, m, d) = epoch_to_ymd(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Subtract `days` from a `YYYY-MM-DD` string, returning a same-shaped
/// `YYYY-MM-DD` string. Used to compute the default lookback bound.
fn date_minus_days(date: &str, days: i64) -> String {
    let secs = parse_ymd_to_epoch(date).unwrap_or(0);
    let earlier = secs.saturating_sub(days * 86_400);
    let (y, m, d) = epoch_to_ymd(earlier);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_ymd_to_epoch(date: &str) -> Option<i64> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    Some(ymd_to_epoch(y, m, d))
}

/// Howard Hinnant civil → days conversion at 00:00:00 UTC.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
fn ymd_to_epoch(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m <= 2 { m + 9 } else { m - 3 };
    let doy = (153 * u64::from(mp) + 2) / 5 + u64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    days * 86_400
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
fn epoch_to_ymd(secs: i64) -> (i32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = (y + i64::from(m <= 2)) as i32;
    (year, m as u32, d as u32)
}

fn render_audit_page(
    from: &str,
    to: &str,
    action_filter: &str,
    entries: &[AuditEntry],
    users: &[User],
) -> String {
    let rows = render_audit_full_rows(entries, users);
    let count = entries.len();
    let truncated_note = if count >= AUDIT_PAGE_LIMIT {
        format!(
            r#"<p class="bnb-meta audit-note">Showing the most recent {AUDIT_PAGE_LIMIT} matches — tighten the date range to see older rows.</p>"#
        )
    } else {
        String::new()
    };
    format!(
        r#"<div data-screen-label="Audit log">
  <header class="page-head">
    <div>
      <div class="bnb-eyebrow">Admin · access</div>
      <h1 class="display acct-h1">Audit log</h1>
      <p class="bnb-meta">Every admin-side mutation lands here. Filter by date range and action prefix.</p>
    </div>
    <a class="action" href="/admin/accounts">← back to accounts</a>
  </header>

  <form method="get" action="/admin/audit" class="bnb-card pad audit-form">
    <label class="audit-field">
      <span class="bnb-meta">From</span>
      <input type="date" name="from" value="{from_esc}" required>
    </label>
    <label class="audit-field">
      <span class="bnb-meta">To</span>
      <input type="date" name="to" value="{to_esc}" required>
    </label>
    <label class="audit-field grow">
      <span class="bnb-meta">Action contains</span>
      <input type="text" name="action" value="{action_esc}"
             placeholder="rule. · settings. · password · …"
             class="audit-action-input">
    </label>
    <button type="submit" class="bnb-btn primary audit-btn-h">Apply</button>
    <a href="/admin/audit" class="bnb-btn ghost audit-btn-h lh">Reset</a>
  </form>

  <section class="bnb-card pad audit-section">
    <div class="bnb-eyebrow audit-count">{count} {pluralised}</div>
    <ol class="account-audit audit-list">
      {rows}
    </ol>
    {truncated_note}
  </section>
</div>"#,
        from_esc = escape_html(from),
        to_esc = escape_html(to),
        action_esc = escape_html(action_filter),
        count = count,
        pluralised = if count == 1 { "entry" } else { "entries" },
    )
}

/// Full per-row layout for the audit page. Wider than the 6-row preview
/// on `/admin/accounts` — surfaces `target` + `metadata` and the full
/// timestamp.
fn render_audit_full_rows(entries: &[AuditEntry], users: &[User]) -> String {
    if entries.is_empty() {
        return String::from(
            r#"<li class="account-audit__empty"><span class="bnb-meta">No matching entries.</span></li>"#,
        );
    }
    let mut out = String::new();
    for e in entries {
        let who = e
            .user_id
            .and_then(|id| users.iter().find(|u| u.id == id))
            .map_or("system", |u| u.username.as_str());
        let target = e
            .target
            .as_ref()
            .map(|t| format!(r#" <span class="target">{}</span>"#, escape_html(t)))
            .unwrap_or_default();
        let metadata = e
            .metadata
            .as_ref()
            .map(|m| {
                format!(
                    r#"<div class="mono bnb-meta audit-meta-row">{}</div>"#,
                    escape_html(m)
                )
            })
            .unwrap_or_default();
        let _ = write!(
            out,
            r#"<li class="audit-row">
  <span class="mono bnb-meta">{when}</span>
  <span class="audit-who">{who}</span>
  <div>
    <span class="audit-action">{action}{target}</span>
    {metadata}
  </div>
</li>"#,
            when = escape_html(&e.at),
            who = escape_html(who),
            action = escape_html(&e.action),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::sqlite;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("birds.db");
        let _conn = sqlite::open_or_create(&db_path).expect("open db");
        // Re-open via AppState's constructor so migrations run.
        let state = AppState::new(db_path).expect("state");
        (dir, state)
    }

    #[test]
    fn render_session_rows_empty() {
        let html = render_session_rows(&[], "");
        assert!(html.contains("No active sessions"));
    }

    #[test]
    fn render_session_rows_marks_current_device() {
        let s = Session {
            id: "sess-current".to_string(),
            user_id: 1,
            issued_at: "2026-05-28 10:00:00".to_string(),
            last_seen: "2026-05-28 10:05:00".to_string(),
            expires_at: "2099-01-01 00:00:00".to_string(),
            user_agent: Some("Firefox 144".to_string()),
            ip_hash: None,
        };
        let s_other = Session {
            id: "sess-other".to_string(),
            ..s.clone()
        };
        let html = render_session_rows(&[s, s_other], "sess-current");
        // The matching row is marked data-current="true" and carries
        // the "This device" pill; the other row stays neutral.
        assert!(html.contains(r#"data-current="true""#));
        assert!(html.contains(r#"data-current="false""#));
        assert!(html.contains("This device"));
    }

    #[test]
    fn render_user_rows_renders_seed_admin_with_reset_only() {
        let (_d, state) = fixture();
        let users = state.with_db(UserStore::list_users).unwrap();
        assert!(!users.is_empty());
        let html = render_user_rows(&users);
        assert!(html.contains("ADMIN"));
        assert!(html.contains("Reset password"));
        // Seed admin cannot be removed: no Remove button on its row.
        assert!(!html.contains(">Remove<"));
    }

    #[test]
    fn render_audit_rows_handles_unknown_user() {
        let entries = vec![AuditEntry {
            id: 1,
            at: "2026-05-28 10:00:00".to_string(),
            user_id: None,
            action: "settings.update".to_string(),
            target: Some("audio".to_string()),
            metadata: None,
        }];
        let html = render_audit_rows(&entries, &[]);
        assert!(html.contains("system"));
        assert!(html.contains("settings.update"));
        assert!(html.contains("audio"));
    }

    #[test]
    fn render_audit_rows_empty() {
        let html = render_audit_rows(&[], &[]);
        assert!(html.contains("No admin actions yet"));
    }

    #[test]
    fn render_body_substitutes_all_placeholders() {
        let (_d, state) = fixture();
        let body = state.with_db(|conn| {
            let admin = conn.find_user_by_name("admin").unwrap();
            let users = conn.list_users().unwrap();
            let sessions = conn.list_sessions(admin.id).unwrap();
            let audit = conn.recent(AUDIT_PREVIEW_LIMIT).unwrap();
            render_body(&admin, &users, &sessions, &audit, "")
        });
        assert!(!body.contains("{{"));
        assert!(body.contains("Accounts &amp; sessions"));
        assert!(body.contains("Administrator"));
    }

    // ------------------------------------------------------------------
    // /admin/audit page tests
    // ------------------------------------------------------------------

    #[test]
    fn ymd_epoch_roundtrip_anchor_dates() {
        // 2000-01-01 = 946684800 (UTC midnight, well-known anchor).
        assert_eq!(ymd_to_epoch(2000, 1, 1), 946_684_800);
        assert_eq!(epoch_to_ymd(946_684_800), (2000, 1, 1));
        // 2024-02-29 (leap) = 1709164800.
        assert_eq!(ymd_to_epoch(2024, 2, 29), 1_709_164_800);
        assert_eq!(epoch_to_ymd(1_709_164_800), (2024, 2, 29));
    }

    #[test]
    fn date_minus_days_handles_month_boundary() {
        // 14 days before 2026-05-10 is 2026-04-26.
        assert_eq!(date_minus_days("2026-05-10", 14), "2026-04-26");
        // 30 days before 2026-03-01 crosses a leap-aware boundary —
        // 30 days back from March 1 lands in late January / Feb 1.
        let earlier = date_minus_days("2026-03-01", 30);
        assert_eq!(earlier, "2026-01-30");
    }

    #[test]
    fn resolve_audit_range_fills_defaults() {
        // Empty params → from = today - 14 days, to = today.
        let p = AuditParams::default();
        let (from, to) = resolve_audit_range(&p);
        // Today should equal the second value.
        assert_eq!(to, today_date_string());
        // From should equal today_minus_14.
        let expected_from = date_minus_days(&today_date_string(), i64::from(AUDIT_DEFAULT_DAYS));
        assert_eq!(from, expected_from);
    }

    #[test]
    fn resolve_audit_range_uses_supplied_bounds() {
        let p = AuditParams {
            from: Some("2026-04-01".to_string()),
            to: Some("2026-04-15".to_string()),
            action: None,
        };
        let (from, to) = resolve_audit_range(&p);
        assert_eq!(from, "2026-04-01");
        assert_eq!(to, "2026-04-15");
    }

    #[test]
    fn render_audit_full_rows_shows_user_action_target_and_metadata() {
        let users = vec![User {
            id: 7,
            username: "jess".to_string(),
            pwd_argon2: String::new(),
            role: Role::Viewer,
            label: None,
            created_at: "2026-01-01".to_string(),
            disabled_at: None,
        }];
        let entries = vec![AuditEntry {
            id: 1,
            at: "2026-05-25 10:30:00".to_string(),
            user_id: Some(7),
            action: "rule.toggle".to_string(),
            target: Some("rule:nightjar".to_string()),
            metadata: Some(r#"{"enabled":false}"#.to_string()),
        }];
        let html = render_audit_full_rows(&entries, &users);
        assert!(html.contains("2026-05-25 10:30:00"));
        assert!(html.contains("jess"));
        assert!(html.contains("rule.toggle"));
        assert!(html.contains("rule:nightjar"));
        // Metadata is escaped (JSON quoted) — verify the quote becomes an entity.
        assert!(html.contains("&quot;enabled&quot;"));
    }

    #[test]
    fn render_audit_full_rows_empty_message() {
        let html = render_audit_full_rows(&[], &[]);
        assert!(html.contains("No matching entries"));
    }

    #[test]
    fn render_audit_page_substitutes_form_values() {
        let html = render_audit_page("2026-05-01", "2026-05-31", "rule", &[], &[]);
        assert!(html.contains(r#"name="from" value="2026-05-01""#));
        assert!(html.contains(r#"name="to" value="2026-05-31""#));
        assert!(html.contains(r#"name="action" value="rule""#));
    }

    #[test]
    fn create_user_password_too_short_is_rejected() {
        let form = CreateUserForm {
            username: "jess".to_string(),
            password: "short".to_string(),
            label: None,
        };
        assert!(form.password.len() < 10);
    }
}
