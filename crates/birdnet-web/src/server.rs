//! Axum server setup and lifecycle.
//!
//! Configures the axum Router with Tower middleware (`CORS`, tracing,
//! rate limiting, a stateless CSRF guard, and response-hardening security
//! headers), mounts API routes, and manages graceful shutdown.

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::routes;
use crate::state::AppState;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Listen address (default: 127.0.0.1:8502).
    pub addr: SocketAddr,
    /// Path to the `SQLite` database.
    pub db_path: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 8502)),
            db_path: PathBuf::from("birds.db"),
        }
    }
}

/// Server errors.
#[derive(Debug)]
pub enum ServerError {
    /// Failed to bind to address.
    Bind(String),
    /// Server runtime error.
    Runtime(String),
    /// Database initialization error.
    Database(String),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(msg) => write!(f, "bind error: {msg}"),
            Self::Runtime(msg) => write!(f, "runtime error: {msg}"),
            Self::Database(msg) => write!(f, "database error: {msg}"),
        }
    }
}

impl std::error::Error for ServerError {}

/// Build the axum application router with all middleware and routes,
/// gating `/admin/*` behind the v2 cookie-session middleware.
///
/// Applies a per-IP token-bucket rate limiter (30 req/s, burst 60) to
/// protect the API from overload.  Static assets and WebSocket connections
/// share the same limit bucket as API calls but are lightweight by nature.
///
/// Admin authentication is cookie-based (the O-14/O-15 wire flip):
/// `CADDY_USER`/`CADDY_PWD` feed the cookie middleware via the env (the
/// bootstrap in `helpers::auth` hashes them into the seed admin row's
/// `pwd_argon2`), and the middleware open-bypasses when no admin password
/// is configured — matching the fresh-Pi "no password = open admin" contract.
pub fn build_router(state: AppState) -> Router {
    // Rate limiter: 30 req/s sustained, 60-request burst per IP.
    build_router_with_rate_limit(state, RateLimitConfig::default())
}

/// Build the router with an explicit rate-limit configuration.
///
/// [`build_router`] uses the shipped default and is what the station runs. This
/// variant exists for the visual-QA fixture (`examples/screenshot_server`),
/// which is deliberately hammered by a machine-speed harness: the sweep loads
/// 152 pages back to back, far above any rate a browser produces. Throttling
/// the harness that exists to hammer this server tests nothing about the
/// product, and turns an unlucky burst into a red build on an asset `429`.
///
/// Measured for contrast, so the default is not adjusted on a hunch: a cold
/// dashboard load is 24 requests, the heaviest page (recordings) 34, and two
/// rapid dashboard loads back to back 48 — all comfortably inside the 60-burst
/// default, with no `429`. Real clients are nowhere near the limit, so the
/// production numbers stay exactly as they were.
pub fn build_router_with_rate_limit(state: AppState, rate_limit: RateLimitConfig) -> Router {
    let limiter = Arc::new(RateLimiter::new(rate_limit));

    // Gate `/admin/*` behind the v2 cookie middleware. The middleware
    // handles the "no admin password configured" bypass internally
    // (matches the pre-flip basic-auth contract).
    let admin = routes::admin_routes();
    let admin = crate::auth_middleware::apply(admin, state.clone());

    let router = routes::public_routes().merge(admin).with_state(state);

    // Layer order is outermost-last. The CSRF guard runs after rate limiting
    // (so request floods are still throttled) and before auth, rejecting
    // cross-site state-changing requests. The security-headers layer is added
    // last so it decorates *every* response — 401/404/429, static files, and
    // handler output alike.
    router
        .layer(axum::middleware::from_fn(
            crate::security::csrf_guard_middleware,
        ))
        .layer(axum::middleware::from_fn(move |req, next| {
            let limiter = Arc::clone(&limiter);
            crate::rate_limit::rate_limit_middleware(limiter, req, next)
        }))
        .layer(TraceLayer::new_for_http())
        .layer(build_cors_layer())
        .layer(axum::middleware::from_fn(
            crate::security::security_headers_middleware,
        ))
}

/// Build the CORS policy.
///
/// Secure by default: a station's own web UI is served from the same origin it
/// calls, so no cross-origin access is needed. Echoing the previous
/// `Access-Control-Allow-Origin: *` let *any* website the operator's browser
/// visited read this station's API over the LAN — so the default now allows
/// **no** cross-origin reads. Set `BIRDNET_CORS_ALLOWED_ORIGINS` (a
/// comma-separated origin list, e.g. `https://dash.example.com`) only when
/// fronting the API from a different origin.
fn build_cors_layer() -> CorsLayer {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    match std::env::var("BIRDNET_CORS_ALLOWED_ORIGINS") {
        Ok(raw) if !raw.trim().is_empty() => {
            let origins: Vec<HeaderValue> = raw
                .split(',')
                .filter_map(|o| o.trim().parse::<HeaderValue>().ok())
                .collect();
            if origins.is_empty() {
                cors
            } else {
                tracing::info!(
                    count = origins.len(),
                    "CORS: allowing configured cross-origin origins"
                );
                cors.allow_origin(origins)
            }
        }
        // No configured origins → same-origin only (no ACAO header emitted).
        _ => cors,
    }
}

/// Start the web server.
///
/// # Errors
///
/// Returns `ServerError` if the server fails to bind or start.
pub async fn start(config: ServerConfig) -> Result<(), ServerError> {
    let state = AppState::new(config.db_path).map_err(|e| ServerError::Database(e.to_string()))?;
    let app = build_router(state);

    tracing::info!(addr = %config.addr, "starting web server");

    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .map_err(|e| ServerError::Bind(e.to_string()))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| ServerError::Runtime(e.to_string()))?;

    tracing::info!("web server stopped");
    Ok(())
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received Ctrl+C"),
        () = terminate => tracing::info!("received SIGTERM"),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_router, build_router_with_rate_limit};
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    async fn burst_hits_429(app: axum::Router, requests: usize) -> bool {
        for _ in 0..requests {
            let req = Request::builder()
                .uri("/static/css/app.css")
                .body(Body::empty())
                .expect("request");
            let resp = app.clone().oneshot(req).await.expect("response");
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                return true;
            }
        }
        false
    }

    /// The shipped router must keep the strict limiter.
    ///
    /// `build_router_with_rate_limit` exists so the visual-QA fixture can opt
    /// out of throttling, which makes it newly possible to hand the *station* a
    /// permissive config and have nothing notice. The limiter's own tests cover
    /// the token bucket; this covers the wiring, which is the part that would
    /// silently regress.
    #[tokio::test]
    async fn the_shipped_router_still_rate_limits() {
        assert!(
            burst_hits_429(build_router(test_state()), 400).await,
            "build_router must apply the default limiter (30 req/s, 60 burst)"
        );
    }

    /// And the opt-out actually opts out — otherwise the fixture change is
    /// cosmetic and the visual-QA sweep stays intermittently red.
    #[tokio::test]
    async fn a_permissive_config_does_not_throttle() {
        let app = build_router_with_rate_limit(
            test_state(),
            crate::rate_limit::RateLimitConfig {
                requests_per_second: 100_000.0,
                burst_capacity: 100_000,
                trust_x_forwarded_for: false,
            },
        );
        assert!(
            !burst_hits_429(app, 400).await,
            "a permissive config must serve a burst the default would reject"
        );
    }
}
