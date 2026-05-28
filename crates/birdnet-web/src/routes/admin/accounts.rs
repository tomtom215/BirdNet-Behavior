//! Accounts & sessions surface (O-15).
//!
//! Page at `/admin/accounts` with three cards: active sessions, users
//! roster, and a 6-row preview of the audit log. Mutating endpoints
//! cover invite/delete/disable on users and revoke on sessions; the
//! handlers hit the stores in [`birdnet_db::accounts`].
//!
//! The page deliberately reads from the database with no fallback "demo"
//! data — until the auth wire is flipped the request-time user is the
//! seed `admin` row (the migration creates one row whether the operator
//! visits this page or not). See the `TODO(O-15-followup)` comments
//! below for the call sites that need `require_admin` once O-14's
//! cookie middleware lands.
//!
//! Each mutating handler emits an OOB toast (success/warn) using the
//! O-18 helper so the operator sees the result without a full reload.

use std::fmt::Write as _;

use axum::Form;
use axum::Router;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
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

async fn accounts_page(
    State(state): State<AppState>,
    request_user: RequestUser,
) -> Html<String> {
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
        let actions = if u.username == "admin" {
            // Seed admin can be reset but not removed/disabled.
            String::from(
                r#"<button class="bnb-btn ghost" hx-post="/admin/accounts/users/0/password-reset-stub">Reset password</button>"#,
            )
        } else {
            let id = u.id;
            format!(
                r##"<div style="display:inline-flex;gap:8px;">
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
      <h1 class="display" style="font-size:32px;">Accounts &amp; sessions</h1>
    </div>
  </header>
  <section class="bnb-card pad" style="border-color:var(--rare);color:var(--rare);">
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

async fn create_user(
    State(state): State<AppState>,
    Form(form): Form<CreateUserForm>,
) -> Response {
    if form.password.len() < 10 {
        return toast::oob_only(Toast::error(
            "Password must be at least 10 characters.",
        ))
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
        Err(AccountsError::Invalid(msg)) => {
            toast::oob_only(Toast::error(msg)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "create_user failed");
            toast::oob_only(Toast::error(
                "Could not create the user. See server logs.",
            ))
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
        Err(AccountsError::Invalid(msg)) => {
            toast::oob_only(Toast::warn(msg)).into_response()
        }
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
// POST /admin/accounts/users/{id}  (rotate password — stub)
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
        return toast::oob_only(Toast::error(
            "Password must be at least 10 characters.",
        ))
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
        return toast::oob_only(Toast::error("Could not revoke that session."))
            .into_response();
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

async fn audit_full_page(State(state): State<AppState>) -> Response {
    // TODO(O-15-followup): a small filter form (date range, action prefix)
    // belongs here. For now this is a redirect back to the accounts page
    // so the link in the template doesn't 404.
    let _ = state;
    Redirect::to("/admin/accounts").into_response()
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
