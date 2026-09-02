//! Branded sign-in page and session management (O-14).
//!
//! Adds `GET /login`, `POST /login`, and `POST /logout` — the cookie path
//! documented in [`crate::session`]. The auth wire is flipped:
//! [`crate::auth_middleware`] is the live gate for `/admin/*`, validating
//! the `bnb-session` cookie this page mints and enforcing the viewer/admin
//! role split on writes.
//!
//! ## Behaviour
//!
//! * `GET /login` renders the branded form (single visit, no redirect).
//! * `POST /login` validates the `CADDY_USER` / `CADDY_PWD` credentials,
//!   then issues `Set-Cookie: bnb-session=…`. Wrong credentials redirect
//!   back to `/login?error=1`.
//! * `POST /logout` clears the cookie and redirects to `/`.
//!
//! Successful sign-in mints a cookie the admin middleware then honours on
//! subsequent requests; signing out clears it.

use axum::Form;
use axum::Router;
use axum::extract::{Extension, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use birdnet_db::accounts::{self, SessionStore, UserStore};

use crate::client_ip::ClientIp;
use crate::routes::pages::escape_html;
use crate::session;
use crate::state::AppState;

const LOGIN_TEMPLATE: &str = include_str!("../../templates/login.html");

/// Mount the sign-in and sign-out routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout_submit))
}

/// `GET /login` — render the branded sign-in form.
async fn login_page(req: Request) -> Html<String> {
    let query = req.uri().query().unwrap_or_default();
    let error = query.split('&').any(|p| p == "error=1");
    let next = query
        .split('&')
        .find_map(|p| p.strip_prefix("next=").map(str::to_string))
        .filter(|s| s.starts_with('/'))
        .unwrap_or_else(|| "/admin/overview".to_string());

    Html(render_login(LoginContext {
        error,
        rate_limited: false,
        next: &next,
    }))
}

/// `POST /login` — verify credentials and issue a v2 session cookie
/// bound to a freshly created row in the `sessions` table.
///
/// Credential verification falls back through three paths so the
/// transition from #89's basic-auth scaffolding is graceful:
///
/// 1. DB lookup: if a user with `form.username` exists and its
///    `pwd_argon2` verifies the submitted password, that's the user.
/// 2. `CADDY_USER`/`CADDY_PWD` env path: legacy basic-auth credentials
///    map onto the seed admin row. This is what makes the wire flip
///    survive the case where the bootstrap hasn't run yet.
/// 3. Anything else → `?error=1`.
async fn login_submit(
    State(state): State<AppState>,
    client: Option<Extension<ClientIp>>,
    headers: axum::http::HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let next = sanitize_next(form.next.as_deref()).to_string();
    let device = DeviceFingerprint::from_request(client.as_deref(), &headers);
    let configured_env = match (std::env::var("CADDY_USER"), std::env::var("CADDY_PWD")) {
        (Ok(u), Ok(p)) if !u.is_empty() && !p.is_empty() => Some((u, p)),
        _ => None,
    };

    // Authenticate. The order matters: DB first, env-fallback second,
    // so an operator who has rotated their password in the UI doesn't
    // hit the env-fallback path with stale credentials.
    let Some(auth_user_id) = authenticate(
        &state,
        &form.username,
        &form.password,
        configured_env.as_ref(),
    ) else {
        // Wrong credentials, or no admin password configured at all.
        // Bypass the gate when basic-auth would also have let the
        // request through (no CADDY_USER + no DB admin password).
        if configured_env.is_none()
            && state
                .with_db(|conn| conn.find_user_by_name("admin"))
                .is_ok_and(|u| accounts::is_legacy_password_hash(&u.pwd_argon2))
        {
            return open_bypass_redirect(&state, &next, &device);
        }
        let query = format!("?error=1&next={}", urlencode_path(&next));
        return Redirect::to(&format!("/login{query}")).into_response();
    };

    let ttl_ms = if form.remember.as_deref() == Some("1") {
        session::REMEMBER_ME_TTL_MS
    } else {
        session::default_ttl_ms()
    };

    // Mint a fresh session id, persist a row, and emit the bound v2 cookie.
    let session_id = session::generate_session_id();
    let expires_at = expires_at_for_ttl(ttl_ms);
    let create_result = state.with_db(|conn| {
        conn.create_session(
            &session_id,
            auth_user_id,
            &expires_at,
            device.user_agent.as_deref(),
            device.ip_hash.as_deref(),
        )
    });
    if let Err(e) = create_result {
        tracing::error!(error = %e, "create_session failed during login");
        let query = format!("?error=1&next={}", urlencode_path(&next));
        return Redirect::to(&format!("/login{query}")).into_response();
    }

    let token = session::issue_token(&session_id, ttl_ms);
    redirect_with_cookie(&token, ttl_ms, &next)
}

/// Verify credentials against (DB row, hash) first, falling back to
/// (`CADDY_USER`, `CADDY_PWD`) env. Returns the user id of the verified
/// row (or the seed admin's id on env-fallback).
fn authenticate(
    state: &AppState,
    username: &str,
    password: &str,
    configured_env: Option<&(String, String)>,
) -> Option<i64> {
    // Path 1: DB lookup.
    if let Ok(user) = state.with_db(|conn| conn.find_user_by_name(username)) {
        if user.disabled_at.is_some() {
            return None;
        }
        let verifies = accounts::verify_password(&user.pwd_argon2, password).unwrap_or(false);
        if verifies {
            // Best-effort: rotate a legacy hash forward on successful
            // sign-in so the next basic-auth-free path doesn't have to.
            if accounts::is_legacy_password_hash(&user.pwd_argon2)
                && let Ok(new_hash) = accounts::hash_password(password)
            {
                let _ = state.with_db(|conn| conn.set_password(user.id, &new_hash));
            }
            return Some(user.id);
        }
    }

    // Path 2: CADDY_USER / CADDY_PWD env fallback. Maps onto the seed
    // admin row's id when both env values are set and match the
    // submitted credentials.
    if let Some((u, p)) = configured_env
        && constant_time_eq(username.as_bytes(), u.as_bytes())
        && constant_time_eq(password.as_bytes(), p.as_bytes())
    {
        return state
            .with_db(|conn| conn.find_user_by_name("admin"))
            .map(|admin| admin.id)
            .ok();
    }

    None
}

/// What we record about the device a session was minted on.
///
/// Both fields exist as columns and both were written as `NULL` at every call
/// site, so `/admin/accounts` listed every session as an anonymous row and the
/// "is one of these not mine?" question the page exists to answer could not be
/// asked. Filling them is the point of resolving a client address correctly:
/// an `ip_hash` taken from the peer would say "the reverse proxy" for every
/// session on every proxied station, which is worse than `NULL` because it
/// looks like an answer.
struct DeviceFingerprint {
    user_agent: Option<String>,
    ip_hash: Option<String>,
}

impl DeviceFingerprint {
    /// `client` is [`ClientIp`] as resolved by the rate-limit layer. It is
    /// `None` only when that layer did not run — a handler called directly in
    /// a test — and then no address is recorded rather than a wrong one.
    fn from_request(client: Option<&ClientIp>, headers: &axum::http::HeaderMap) -> Self {
        Self {
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.chars().take(255).collect()),
            ip_hash: client.map(|c| session::hash_client_ip(c.0)),
        }
    }
}

/// "Anyone can sign in" bypass — issued only when neither `CADDY_PWD` nor
/// a DB-stored admin password is configured. Mirrors the basic-auth
/// shape from #89: the surface is reachable on a freshly provisioned
/// station before the operator has chosen a password.
fn open_bypass_redirect(state: &AppState, next: &str, device: &DeviceFingerprint) -> Response {
    let session_id = session::generate_session_id();
    let Ok(admin) = state.with_db(|conn| conn.find_user_by_name("admin")) else {
        return Redirect::to(next).into_response();
    };
    let admin_id = admin.id;
    let expires_at = expires_at_for_ttl(session::default_ttl_ms());
    let _ = state.with_db(|conn| {
        conn.create_session(
            &session_id,
            admin_id,
            &expires_at,
            device.user_agent.as_deref(),
            device.ip_hash.as_deref(),
        )
    });
    let token = session::issue_token(&session_id, session::default_ttl_ms());
    redirect_with_cookie(&token, session::default_ttl_ms(), next)
}

/// Compute the `sessions.expires_at` value for a given ttl. Stored as a
/// SQLite `datetime('now', '+N seconds')` text so comparisons line up
/// with the `WHERE expires_at > datetime('now')` clause in
/// `SessionStore::list_sessions`.
fn expires_at_for_ttl(ttl_ms: u64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let secs = i64::try_from(ttl_ms / 1000).unwrap_or(i64::MAX);
    let target = now_secs.saturating_add(secs);
    format_sqlite_datetime(target)
}

/// Format epoch seconds as SQLite's `YYYY-MM-DD HH:MM:SS` UTC. Avoids
/// pulling chrono in — same hand-roll as the weather poll's cutoff.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
fn format_sqlite_datetime(secs: i64) -> String {
    let rem = secs.rem_euclid(86_400);
    let (year, m, d) = birdnet_core::civil::civil_from_days(secs.div_euclid(86_400));
    let hh = (rem / 3600) as u32;
    let mm = ((rem % 3600) / 60) as u32;
    let ss = (rem % 60) as u32;
    format!("{year:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

/// `POST /logout` — revoke the bound session row (if any) and clear
/// the cookie.
async fn logout_submit(State(state): State<AppState>, req: Request) -> Response {
    // Pull the cookie from the request and revoke the bound session
    // before clearing the browser-side cookie. Failures are logged but
    // never block the redirect — the cookie still gets cleared.
    if let Some(token) = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(session::extract_token)
        && let Some(validated) = session::validate_token(token)
    {
        let _ =
            state.with_db(|conn| <_ as SessionStore>::revoke_session(conn, &validated.session_id));
    }

    let mut resp = Redirect::to("/").into_response();
    let public_url = std::env::var("BNB_PUBLIC_URL").ok();
    let clear = session::build_clear_cookie(public_url.as_deref());
    if let Ok(val) = HeaderValue::from_str(&clear) {
        resp.headers_mut().append(header::SET_COOKIE, val);
    }
    resp
}

fn redirect_with_cookie(token: &str, ttl_ms: u64, next: &str) -> Response {
    let mut resp = Redirect::to(next).into_response();
    let public_url = std::env::var("BNB_PUBLIC_URL").ok();
    let set_cookie = session::build_set_cookie(token, ttl_ms, public_url.as_deref());
    if let Ok(val) = HeaderValue::from_str(&set_cookie) {
        resp.headers_mut().append(header::SET_COOKIE, val);
    }
    *resp.status_mut() = StatusCode::SEE_OTHER;
    resp
}

fn sanitize_next(raw: Option<&str>) -> &str {
    // Only allow path-rooted redirects so an attacker can't smuggle in an
    // off-host URL via `next=`. Anything else falls back to the admin
    // overview.
    raw.filter(|s| s.starts_with('/') && !s.starts_with("//"))
        .unwrap_or("/admin/overview")
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    username: String,
    password: String,
    #[serde(default)]
    remember: Option<String>,
    #[serde(default)]
    next: Option<String>,
}

#[derive(Clone, Copy)]
struct LoginContext<'a> {
    error: bool,
    rate_limited: bool,
    next: &'a str,
}

fn render_login(ctx: LoginContext<'_>) -> String {
    let station = std::env::var("BIRDNET_SITENAME").unwrap_or_else(|_| "station".to_string());
    let version = env!("CARGO_PKG_VERSION");
    let (error_class, error_body) = if ctx.rate_limited {
        (
            "is-visible",
            "Too many attempts. Try again in 30 s.".to_string(),
        )
    } else if ctx.error {
        ("is-visible", "Incorrect username or password.".to_string())
    } else {
        ("", String::new())
    };
    let disabled = if ctx.rate_limited { " disabled" } else { "" };

    LOGIN_TEMPLATE
        .replace("{{title}}", "Sign in")
        .replace("{{station}}", &escape_html(&station))
        .replace("{{error_class}}", error_class)
        .replace("{{error_body}}", &escape_html(&error_body))
        .replace("{{next_path}}", &escape_html(ctx.next))
        .replace("{{rate_limited}}", disabled)
        .replace("{{version}}", version)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

fn urlencode_path(s: &str) -> String {
    crate::urls::encode_path(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_login_substitutes_placeholders() {
        let html = render_login(LoginContext {
            error: true,
            rate_limited: false,
            next: "/admin/overview",
        });
        assert!(html.contains("is-visible"));
        assert!(html.contains("Incorrect username or password."));
        // The recovery hint must survive template edits: without it, an
        // operator locked out by the /admin closure has no on-screen route to
        // the credential, only a root grep they have no reason to know about.
        assert!(
            html.contains("Forgotten the password?"),
            "login page must offer a recovery route"
        );
        assert!(
            html.contains("CADDY_PWD") && html.contains("/etc/birdnet/birdnet.conf"),
            "the hint must name where the password actually lives"
        );
        assert!(
            html.contains("<code>admin</code>"),
            "the hint must name the username, which is not the one the installer used to print"
        );
        assert!(html.contains(r#"value="/admin/overview""#));
        assert!(!html.contains("{{"));
    }

    #[test]
    fn render_login_rate_limited_disables_submit() {
        let html = render_login(LoginContext {
            error: true,
            rate_limited: true,
            next: "/admin/overview",
        });
        assert!(html.contains("Too many attempts"));
        assert!(html.contains("disabled"));
    }

    #[test]
    fn render_login_clean_state_hides_alert() {
        let html = render_login(LoginContext {
            error: false,
            rate_limited: false,
            next: "/admin/overview",
        });
        // The alert element is always rendered; the `is-visible` modifier
        // class is what makes it visible. Look at the rendered class attribute
        // rather than substring-matching "is-visible" (which also appears in
        // the template's HTML comments documenting the slot).
        assert!(html.contains(r#"class="login-alert ""#));
        assert!(!html.contains(r#"class="login-alert is-visible""#));
        assert!(!html.contains("{{"));
    }

    #[test]
    fn urlencode_preserves_path_chars() {
        assert_eq!(urlencode_path("/admin/overview"), "/admin/overview");
        assert_eq!(urlencode_path("/r?x=1"), "/r%3Fx%3D1");
    }

    #[test]
    fn constant_time_eq_matches_pairs() {
        assert!(constant_time_eq(b"admin", b"admin"));
        assert!(!constant_time_eq(b"admin", b"Admin"));
        assert!(!constant_time_eq(b"admin", b"adminx"));
    }

    #[test]
    fn sanitize_next_blocks_external_redirects() {
        assert_eq!(sanitize_next(Some("/admin/overview")), "/admin/overview");
        assert_eq!(sanitize_next(Some("/r/abc")), "/r/abc");
        assert_eq!(sanitize_next(Some("//evil.example/x")), "/admin/overview");
        assert_eq!(
            sanitize_next(Some("https://evil.example")),
            "/admin/overview"
        );
        assert_eq!(sanitize_next(None), "/admin/overview");
    }
}
