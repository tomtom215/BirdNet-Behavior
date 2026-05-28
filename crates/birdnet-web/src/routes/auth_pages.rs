//! Branded sign-in page and session management (O-14).
//!
//! Adds `GET /login`, `POST /login`, and `POST /logout`. These plumb the
//! cookie path documented in [`crate::session`] without flipping the
//! auth wire: the basic-auth middleware in [`crate::auth`] still gates
//! `/admin/*`. The cookie route is ready for the maintainer to enable
//! once the open RFC questions in `docs/proposed_changes/O-14_login/DIFF.md`
//! are resolved — see the `TODO(O-14-followup)` markers.
//!
//! ## Verification
//!
//! Until the wire is flipped:
//! * `GET /login` renders the branded form (single visit, no redirect).
//! * `POST /login` validates the same `CADDY_USER` / `CADDY_PWD` credentials
//!   the Basic Auth middleware reads, then issues `Set-Cookie: bnb-session=…`.
//!   Wrong credentials redirect back to `/login?error=1`.
//! * `POST /logout` clears the cookie and redirects to `/`.
//!
//! Successful sign-in is observable as a freshly minted cookie in the
//! response headers; the browser then carries it on subsequent requests
//! but the existing middleware ignores it.

use axum::Form;
use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use crate::routes::pages::escape_html;
use crate::session;
use crate::state::AppState;

const LOGIN_TEMPLATE: &str = include_str!("../../templates/login.html");

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

/// `POST /login` — verify credentials and issue a session cookie.
///
/// TODO(O-14-followup): once the session-cookie shape is signed off
/// (see RFC in `docs/proposed_changes/O-14_login/DIFF.md`), wire the
/// auth middleware to validate the cookie instead of `Authorization:
/// Basic`. Until then this issues a cookie that the existing middleware
/// will ignore — the credential check still passes through the Basic
/// Auth path on the next `/admin/*` request.
async fn login_submit(Form(form): Form<LoginForm>) -> Response {
    let (configured_user, configured_pwd) =
        match (std::env::var("CADDY_USER"), std::env::var("CADDY_PWD")) {
            (Ok(u), Ok(p)) if !u.is_empty() && !p.is_empty() => (u, p),
            _ => {
                // No admin password configured — Basic Auth would let
                // everyone through too, so the same defaults apply here.
                let next = sanitize_next(form.next.as_deref());
                return redirect_with_cookie(
                    &session::issue_token(session::default_ttl_ms()),
                    session::default_ttl_ms(),
                    next,
                );
            }
        };

    if !credentials_match(&form.username, &configured_user, &form.password, &configured_pwd)
    {
        let next = sanitize_next(form.next.as_deref());
        let query = format!("?error=1&next={}", urlencode_path(next));
        return Redirect::to(&format!("/login{query}")).into_response();
    }

    let ttl_ms = if form.remember.as_deref() == Some("1") {
        session::REMEMBER_ME_TTL_MS
    } else {
        session::default_ttl_ms()
    };
    let token = session::issue_token(ttl_ms);
    let next = sanitize_next(form.next.as_deref());
    redirect_with_cookie(&token, ttl_ms, next)
}

/// `POST /logout` — clear the cookie and redirect to the dashboard.
async fn logout_submit() -> Response {
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

fn credentials_match(user: &str, expected_user: &str, pwd: &str, expected_pwd: &str) -> bool {
    constant_time_eq(user.as_bytes(), expected_user.as_bytes())
        && constant_time_eq(pwd.as_bytes(), expected_pwd.as_bytes())
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
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
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
        assert_eq!(sanitize_next(Some("https://evil.example")), "/admin/overview");
        assert_eq!(sanitize_next(None), "/admin/overview");
    }
}
