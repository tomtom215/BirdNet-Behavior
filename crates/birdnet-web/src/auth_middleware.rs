//! Cookie-based auth middleware for the O-14 / O-15 wire flip.
//!
//! Sits in front of the `/admin/*` Router. Validates the v2 `bnb-session`
//! cookie, looks up the session row, and attaches a `RequestUser` to
//! the request extensions so handlers can read the authenticated identity
//! via the [`RequestUser`] extractor.
//!
//! ## Behaviour
//!
//! * Excluded paths (health checks, the detection WebSocket) bypass auth —
//!   see the `is_excluded` helper below.
//! * No `bnb-session` cookie or an unparseable / expired token →
//!   303 redirect to `/login?next=<original-path>`. POST / PATCH /
//!   DELETE under `/admin/*` falls back to 401 rather than a redirect
//!   because htmx doesn't follow `303`s on writes.
//! * Cookie validates but the bound session row is missing (revoked
//!   via the accounts UI) or expired → same as above.
//! * Cookie validates → attach the [`RequestUser`] and touch `last_seen`
//!   on the session row.
//!
//! ## "No admin password configured" bypass
//!
//! Preserves the #89 open-by-default contract: if neither `CADDY_PWD`
//! is set nor the seed admin row carries a real password hash, the
//! middleware lets the request through unauthenticated and attaches a
//! synthetic `RequestUser` mapped onto the seed admin id. Matches the
//! fresh-Pi "no password = open admin" contract.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use birdnet_db::accounts::{self, Role, SessionStore, User, UserStore};

use crate::session;
use crate::state::AppState;

/// Identity attached to every request that passes the cookie middleware.
/// Handlers extract it via `Extension<RequestUser>` or via the
/// [`RequestUser`] axum extractor below.
#[derive(Debug, Clone)]
pub struct RequestUser {
    /// Authenticated user record loaded from the `users` table.
    pub user: User,
    /// Session token bound to this request (matches the `sessions.id` row).
    pub session_id: String,
}

impl RequestUser {
    /// Convenience: are we the admin role?
    #[must_use]
    pub const fn is_admin(&self) -> bool {
        matches!(self.user.role, Role::Admin)
    }
}

/// Axum extractor — pulls the [`RequestUser`] out of request extensions.
/// Handlers that need the authenticated identity take this as a parameter.
impl<S> FromRequestParts<S> for RequestUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().cloned().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "request user missing — middleware not wired?",
        ))
    }
}

/// Per-route role check. Returns `Ok` for an admin user, `Err(403)` for
/// a viewer. Mutating `/admin/*` handlers call this at the top.
///
/// `Response` is large enough that clippy flags the `Err` arm; the box
/// keeps the result type compact without changing the call shape.
///
/// # Errors
///
/// Returns a `403` response when the role is `Viewer`.
pub fn require_admin(user: &RequestUser) -> Result<(), Box<Response>> {
    if user.is_admin() {
        return Ok(());
    }
    Err(Box::new(forbidden_response()))
}

fn forbidden_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<p>Forbidden — admin role required for this action.</p>",
    )
        .into_response()
}

/// Apply the cookie auth layer to an admin `Router`. Server callers do:
///
/// ```ignore
/// let admin = auth_middleware::apply(admin_router, state.clone());
/// ```
///
/// Returning a wrapped `Router` (vs a freestanding `Layer`) lets the
/// `Arc<AppState>` capture happen at the call site and keeps the
/// `FromFnLayer`'s stable-but-unnameable closure type out of the public
/// surface.
pub fn apply(admin: axum::Router<AppState>, state: AppState) -> axum::Router<AppState> {
    let shared = Arc::new(state);
    admin.layer(axum::middleware::from_fn(
        move |req: Request<Body>, next: Next| {
            let shared = Arc::clone(&shared);
            let fut: AuthFuture =
                Box::pin(async move { cookie_auth_middleware(req, next, &shared).await });
            fut
        },
    ))
}

/// Boxed-future type alias for the middleware closure.
type AuthFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>;

async fn cookie_auth_middleware(request: Request<Body>, next: Next, state: &AppState) -> Response {
    let path = request.uri().path();
    let original_path = request
        .uri()
        .path_and_query()
        .map_or_else(|| path.to_string(), ToString::to_string);

    // Excluded paths (health checks, websocket detection stream).
    if is_excluded(path) {
        return next.run(request).await;
    }

    // No admin password configured → open access. Synthesise a
    // RequestUser pointing at the seed admin so downstream handlers
    // still see an identity.
    if !admin_password_configured(state)
        && let Some(synth) = synthesise_seed_admin(state)
    {
        let mut req = request;
        req.extensions_mut().insert(synth);
        return next.run(req).await;
    }
    // If the bypass branch above didn't trigger (real password
    // configured, or no admin row yet) fall through to the
    // cookie-validation path.

    let token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(session::extract_token);

    let Some(validated) = token.and_then(session::validate_token) else {
        return redirect_to_login(&request, &original_path);
    };

    let lookup = state.with_db(|conn| -> Result<RequestUser, accounts::AccountsError> {
        let session = conn.find_active_session(&validated.session_id)?;
        // Touch last_seen; failure is non-fatal (the auth still succeeds).
        let _ = SessionStore::touch_session(conn, &validated.session_id);
        let user = conn.find_user(session.user_id)?;
        if user.disabled_at.is_some() {
            return Err(accounts::AccountsError::Invalid(
                "user disabled".to_string(),
            ));
        }
        Ok(RequestUser {
            user,
            session_id: validated.session_id,
        })
    });

    let user = match lookup {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(error = %e, "cookie auth: session/user lookup failed");
            return redirect_to_login(&request, &original_path);
        }
    };

    // Authorization: viewers are read-only on `/admin`. Every unsafe method
    // (POST / PATCH / DELETE / PUT) is gated through `require_admin`; safe
    // methods (GET / HEAD) stay open to any authenticated user. Centralising
    // the check here means a newly-added mutating handler is write-gated by
    // default instead of relying on each one to call `require_admin` (O-15
    // RBAC). Every `/admin` state change uses a non-safe method, so this
    // cleanly separates reads from writes.
    if !request.method().is_safe()
        && let Err(resp) = require_admin(&user)
    {
        tracing::info!(
            user = %user.user.username,
            method = %request.method(),
            path = %path,
            "RBAC: viewer denied admin write"
        );
        return *resp;
    }

    let mut req = request;
    req.extensions_mut().insert(user);
    next.run(req).await
}

fn is_excluded(path: &str) -> bool {
    matches!(path, "/api/v2/health" | "/api/v2/ws/detections") || path.starts_with("/api/v2/ws/")
}

fn admin_password_configured(state: &AppState) -> bool {
    // Either CADDY_PWD env is set OR the seed admin row carries a real
    // password hash. The bootstrap in `helpers::auth` keeps these in sync.
    let env_set = std::env::var("CADDY_PWD").is_ok_and(|v| !v.is_empty());
    if env_set {
        return true;
    }
    state
        .with_db(|conn| conn.find_user_by_name("admin"))
        .is_ok_and(|u| !accounts::is_legacy_password_hash(&u.pwd_argon2))
}

fn synthesise_seed_admin(state: &AppState) -> Option<RequestUser> {
    state
        .with_db(|conn| conn.find_user_by_name("admin"))
        .ok()
        .map(|user| RequestUser {
            user,
            // The synthetic session id is a fixed sentinel so audit logs
            // can distinguish "open-bypass" requests from real sessions.
            // It is never written to the sessions table.
            session_id: "open-bypass".to_string(),
        })
}

fn redirect_to_login(request: &Request<Body>, original_path: &str) -> Response {
    // For non-GET methods, htmx doesn't follow 303s on writes — return
    // 401 with an `HX-Redirect` header so client-side htmx can hop us
    // back to /login. Plain GETs get a real 303.
    if request.method().is_safe() {
        let next = urlencode_path(original_path);
        return Redirect::to(&format!("/login?next={next}")).into_response();
    }

    let next = urlencode_path(original_path);
    let target = format!("/login?next={next}");
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::HeaderName::from_static("hx-redirect"), &target),
            (axum::http::HeaderName::from_static("location"), &target),
        ],
        "<p>Sign in required.</p>",
    )
        .into_response()
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
    fn excluded_paths_match_basic_auth() {
        assert!(is_excluded("/api/v2/health"));
        assert!(is_excluded("/api/v2/ws/detections"));
        assert!(is_excluded("/api/v2/ws/spectrogram"));
        assert!(!is_excluded("/admin/overview"));
        assert!(!is_excluded("/"));
    }

    #[test]
    fn urlencode_preserves_admin_path() {
        assert_eq!(urlencode_path("/admin/overview"), "/admin/overview");
    }

    #[test]
    fn forbidden_response_is_403() {
        let resp = forbidden_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    fn fake_request_user(role: Role) -> RequestUser {
        RequestUser {
            user: User {
                id: 1,
                username: "tester".to_string(),
                pwd_argon2: String::new(),
                role,
                label: None,
                created_at: "2026-05-29".to_string(),
                disabled_at: None,
            },
            session_id: "sess_test".to_string(),
        }
    }

    #[test]
    fn require_admin_allows_admin_denies_viewer() {
        assert!(require_admin(&fake_request_user(Role::Admin)).is_ok());
        let denied = require_admin(&fake_request_user(Role::Viewer));
        let resp = denied.expect_err("a viewer must be denied admin actions");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn admin_write_gate_targets_unsafe_methods() {
        use axum::http::Method;
        // The /admin write-gate fires on `!method.is_safe()`: reads stay
        // open to any authenticated user (admin or viewer); writes require
        // the admin role. Locks in the read/write classification the
        // middleware relies on so a future http-crate bump can't silently
        // re-class a method.
        assert!(Method::GET.is_safe());
        assert!(Method::HEAD.is_safe());
        assert!(!Method::POST.is_safe());
        assert!(!Method::PATCH.is_safe());
        assert!(!Method::DELETE.is_safe());
        assert!(!Method::PUT.is_safe());
    }
}
